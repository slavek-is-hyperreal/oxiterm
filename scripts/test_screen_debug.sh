#!/bin/bash
set -x

PROJECT_ROOT="/my_data/oxiterm"
IMAGES_DIR="$PROJECT_ROOT/docs/images"
mkdir -p "$IMAGES_DIR"

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

page_path="examples/hello.thtml"
page_name="hello"

# Start server
./target/debug/oxiterm-cli serve --port 8022 --no-auth "$page_path" > "/tmp/oxiterm_doc_${page_name}.log" 2>&1 &
SERVER_PID=$!
sleep 1.0

# Spawn kitty
kitty -o remember_window_size=no -o initial_window_width=90c -o initial_window_height=26c --title "OxiTerm-Doc-Capture" ssh -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null -p 8022 localhost &
TERM_PID=$!

echo "Sleeping 8 seconds..."
sleep 8.0

# Take screenshot of the entire root screen
import -window root "$IMAGES_DIR/entire_screen.png"
echo "Saved entire screen: docs/images/entire_screen.png"
ls -lh "$IMAGES_DIR/entire_screen.png"

# Close kitty
pkill -f "kitty --title OxiTerm-Doc-Capture" || true

kill $SERVER_PID || true
wait $SERVER_PID 2>/dev/null || true
SERVER_PID=""

echo "Debug complete!"
