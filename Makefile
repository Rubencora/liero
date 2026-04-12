# OpenLiero — top-level Makefile
#
# Builds and runs both the C++ engine and the Rust rewrite side-by-side.
# Both use the same TC directory: TC/openliero
#
# Targets:
#   make rust        — build Rust binary (liero-desktop)
#   make cpp         — build C++ binary (CMake Release)
#   make all         — build both
#   make run-rust    — build + launch Rust binary
#   make run-cpp     — build + launch C++ binary
#   make test-rust   — run replay-diff determinism tests
#   make web         — build WASM package (requires wasm-pack)
#   make serve-web   — build WASM + serve on http://localhost:8080
#   make clean-rust  — cargo clean
#   make clean-cpp   — remove CMake build directory

.PHONY: all rust cpp run-rust run-cpp test-rust web serve-web clean-rust clean-cpp package-macos package-linux deploy-web

RUST_DIR    := openliero-rs
RUST_BIN    := $(RUST_DIR)/target/release/openliero
CPP_BUILD   := build
CPP_BIN     := $(CPP_BUILD)/openliero.app/Contents/MacOS/openliero
TC          := TC/openliero
WEB_CRATE   := $(RUST_DIR)/crates/liero-web
WEB_OUT     := web/pkg

# ── Build ─────────────────────────────────────────────────────────────────────

all: rust cpp

rust:
	cargo build --release --manifest-path $(RUST_DIR)/Cargo.toml -p liero-desktop

cpp:
	@mkdir -p $(CPP_BUILD)
	cmake -S . -B $(CPP_BUILD) -DCMAKE_BUILD_TYPE=Release -DCMAKE_EXPORT_COMPILE_COMMANDS=ON 2>/dev/null || true
	cmake --build $(CPP_BUILD) --target openliero -j$(shell sysctl -n hw.logicalcpu 2>/dev/null || nproc)

# ── Run ───────────────────────────────────────────────────────────────────────

run-rust: rust
	$(RUST_BIN) $(TC)

run-cpp: cpp
	$(CPP_BIN)

# ── Web (WASM) ────────────────────────────────────────────────────────────────

web:
	wasm-pack build $(WEB_CRATE) --target web --out-dir ../../../$(WEB_OUT)

serve-web: web
	@echo "Serving at http://localhost:8080 — Ctrl-C to stop"
	cd web && python3 -m http.server 8080

# ── Test ──────────────────────────────────────────────────────────────────────

test-rust:
	cargo test --manifest-path $(RUST_DIR)/Cargo.toml

# ── Clean ─────────────────────────────────────────────────────────────────────

clean-rust:
	cargo clean --manifest-path $(RUST_DIR)/Cargo.toml

clean-cpp:
	rm -rf $(CPP_BUILD)

# ── Packaging ─────────────────────────────────────────────────────────────────

package-macos: rust
	@echo "→ Assembling OpenLiero.app …"
	@rm -rf OpenLiero.app OpenLiero-macOS.zip
	@mkdir -p OpenLiero.app/Contents/MacOS
	@mkdir -p OpenLiero.app/Contents/Resources/TC
	cp $(RUST_BIN) OpenLiero.app/Contents/MacOS/openliero
	cp pkg/liero.icns OpenLiero.app/Contents/Resources/
	cp -r $(TC) OpenLiero.app/Contents/Resources/TC/openliero
	@printf '<?xml version="1.0" encoding="UTF-8"?>\n\
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN"\n\
  "http://www.apple.com/DTDs/PropertyList-1.0.dtd">\n\
<plist version="1.0"><dict>\n\
  <key>CFBundleName</key>           <string>OpenLiero</string>\n\
  <key>CFBundleDisplayName</key>    <string>OpenLiero</string>\n\
  <key>CFBundleIdentifier</key>     <string>com.openliero.game</string>\n\
  <key>CFBundleVersion</key>        <string>1</string>\n\
  <key>CFBundleShortVersionString</key> <string>1.0</string>\n\
  <key>CFBundleExecutable</key>     <string>openliero</string>\n\
  <key>CFBundleIconFile</key>       <string>liero.icns</string>\n\
  <key>CFBundlePackageType</key>    <string>APPL</string>\n\
  <key>NSHighResolutionCapable</key><true/>\n\
  <key>LSMinimumSystemVersion</key> <string>12.0</string>\n\
</dict></plist>\n' > OpenLiero.app/Contents/Info.plist
	zip -r OpenLiero-macOS.zip OpenLiero.app
	@echo "✓ OpenLiero-macOS.zip ready"

package-linux: rust
	@echo "→ Assembling Linux package …"
	@rm -rf dist/openliero OpenLiero-linux.tar.gz
	@mkdir -p dist/openliero/TC
	cp $(RUST_BIN) dist/openliero/openliero
	cp -r $(TC) dist/openliero/TC/openliero
	tar czf OpenLiero-linux.tar.gz -C dist openliero
	@echo "✓ OpenLiero-linux.tar.gz ready"

# ── Deploy ────────────────────────────────────────────────────────────────────

VPS_HOST    := root@5.78.121.167
VPS_WEBROOT := /var/www/liero

deploy-web:
	rsync -avz --delete web/landing/ $(VPS_HOST):$(VPS_WEBROOT)/
	ssh $(VPS_HOST) 'nginx -t && systemctl reload nginx'
	@echo "✓ Deployed to https://liero.byruben.io"
