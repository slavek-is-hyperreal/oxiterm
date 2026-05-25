#!/bin/bash
# scripts/generate_docs_screenshots.sh
# Automatyczne generowanie zrzutów ekranu dla dokumentacji OxiTerm przy użyciu terminala Kitty.

set -e

PROJECT_ROOT="/my_data/oxiterm"
IMAGES_DIR="$PROJECT_ROOT/docs/images"
mkdir -p "$IMAGES_DIR"

echo "Checking required system tools..."
for tool in kitty xdotool scrot; do
    if ! command -v $tool &> /dev/null; then
        echo "Error: $tool is not installed. Please install it."
        exit 1
    fi
done

echo "Building oxiterm-cli..."
cargo build --bin oxiterm-cli --features oxiterm-server/web

# Cleanup trap to kill any leftover processes
SERVER_PID=""
TERM_PID=""
cleanup() {
    echo "Cleaning up processes..."
    if [ -n "$SERVER_PID" ]; then
        kill $SERVER_PID 2>/dev/null || true
    fi
    pkill -f "kitty --title OxiTerm-Doc-Capture" || true
}
trap cleanup EXIT

# Pliki do przetestowania
PAGES=(
    "examples/hello.thtml"
    "examples/counter.thtml"
    "examples/state_demo.thtml"
    "examples/styles_demo.thtml"
    "examples/navigation_demo.thtml"
    "examples/media_demo.thtml"
    "examples/video_demo.thtml"
    "examples/input_demo.thtml"
    "examples/todo.thtml"
    "examples/showcase.thtml"
)

for page_path in "${PAGES[@]}"; do
    if [ ! -f "$page_path" ]; then
        echo "File $page_path not found! Skipping..."
        continue
    fi

    page_name=$(basename "$page_path" .thtml)
    echo "Generating screenshot for: $page_name..."

    # Start oxiterm-server on port 8022
    ./target/debug/oxiterm-cli serve --port 8022 --no-auth "$page_path" > "/tmp/oxiterm_doc_${page_name}.log" 2>&1 &
    SERVER_PID=$!

    # Wait for server to bind
    sleep 1.0

    # Get active window IDs of kitty before spawning
    BEFORE_WIDS=$(xdotool search --class "kitty" 2>/dev/null || true)

    # Spawn kitty connecting to localhost:8022
    kitty -o remember_window_size=no -o initial_window_width=90c -o initial_window_height=26c --title "OxiTerm-Doc-Capture" ssh -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null -p 8022 localhost &
    TERM_PID=$!

    # Wait for window to appear and render
    sleep 8.0

    # Get active window IDs of kitty after spawning
    AFTER_WIDS=$(xdotool search --class "kitty" 2>/dev/null || true)

    # Find the new window ID
    WID=""
    for id in $AFTER_WIDS; do
        if ! echo "$BEFORE_WIDS" | grep -q "$id"; then
            WID=$id
            break
        fi
    done

    # Fallback to get active window
    if [ -z "$WID" ]; then
        WID=$(xdotool getactivewindow 2>/dev/null || true)
    fi

    if [ -n "$WID" ]; then
        # Focus/activate window
        xdotool windowactivate "$WID"
        xdotool windowfocus "$WID"
        sleep 1.0

        # Take screenshot of the window
        scrot -u "$IMAGES_DIR/${page_name}.png"
        echo "Saved: docs/images/${page_name}.png"

        # Close terminal window
        xdotool windowkill "$WID" || kill $TERM_PID || true
    else
        echo "Error: Failed to find window for $page_name"
    fi

    # Kill server
    kill $SERVER_PID || true
    wait $SERVER_PID 2>/dev/null || true
    SERVER_PID=""
    sleep 0.5
done

echo "Done! Screenshots generated in $IMAGES_DIR/"
