set windows-shell := ["powershell.exe"]
export RUST_BACKTRACE := "1"

# Displays the list of available commands
@just:
    just --list

# Installs the tools pinned in mise.toml (node, rust, wasm-bindgen, wasm-opt)
init:
    mise install

# Installs the web dependencies
install:
    cd web; npm install

# Builds the wasm module, generates the web bindings (target web), and optimizes
build:
    cargo build --release --target wasm32-unknown-unknown
    wasm-bindgen --target web --out-dir web/src/wasm --out-name bevy_worker target/wasm32-unknown-unknown/release/bevy_worker.wasm
    wasm-opt -Oz web/src/wasm/bevy_worker_bg.wasm -o web/src/wasm/bevy_worker_bg.wasm

# Builds the wasm module, then serves the app at http://localhost:5173
run: build install
    cd web; npm run dev

# Serves the already-built app without rebuilding the wasm module
serve:
    cd web; npm run dev

# Produces a production web bundle in web/dist
dist: build install
    cd web; npm run build

# Runs cargo check (wasm target) and a format check
check:
    cargo check --target wasm32-unknown-unknown
    cargo fmt --all -- --check

# Runs clippy for the wasm target and denies warnings
lint:
    cargo clippy --target wasm32-unknown-unknown -- -D warnings

# Formats the code
format:
    cargo fmt --all

# Removes build artifacts (Windows)
[windows]
clean:
    cargo clean
    Remove-Item -Recurse -Force web/dist, web/src/wasm, web/node_modules -ErrorAction SilentlyContinue

# Removes build artifacts (Unix)
[unix]
clean:
    cargo clean
    rm -rf web/dist web/src/wasm web/node_modules
