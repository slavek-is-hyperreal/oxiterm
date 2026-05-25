#!/bin/bash
set -x

PROJECT_ROOT="/my_data/oxiterm"
IMAGES_DIR="$PROJECT_ROOT/docs/images"
mkdir -p "$IMAGES_DIR"

# Build oxiterm-cli
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

page_path="examples/hello.thtml"
page_name="hello"

# Start oxiterm-server on port 8022
./target/debug/oxiterm-cli serve --port 8022 --no-auth "$page_path" > "/tmp/oxiterm_doc_${page_name}.log" 2>&1 &
SERVER_PID=$!

# Wait for server to bind
sleep 1.0

# Get active window IDs of kitty before spawning
BEFORE_WIDS=$(xdotool search --class "kitty" 2>/dev/null || true)
echo "BEFORE_WIDS: $BEFORE_WIDS"

# Spawn kitty connecting to localhost:8022
kitty -o remember_window_size=no -o initial_window_width=90c -o initial_window_height=26c --title "OxiTerm-Doc-Capture" ssh -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null -p 8022 localhost &
TERM_PID=$!

# Wait for window to appear and render
echo "Sleeping 8 seconds to let kitty render..."
sleep 8.0

# Get active window IDs of kitty after spawning
AFTER_WIDS=$(xdotool search --class "kitty" 2>/dev/null || true)
echo "AFTER_WIDS: $AFTER_WIDS"

# Find the new window ID
WID=""
for id in $AFTER_WIDS; do
    if ! echo "$BEFORE_WIDS" | grep -q "$id"; then
        WID=$id
        break
    fi
done
echo "New Kitty WID: $WID"

if [ -n "$WID" ]; then
    # Focus/activate window just in case
    xdotool windowactivate "$WID"
    xdotool windowfocus "$WID"
    sleep 1.5

    # Get details of the active window
    xdotool getwindowname "$WID" || true

    # Take screenshot of the window using ImageMagick import
    import -window "$WID" "$IMAGES_DIR/${page_name}.png"
    echo "Saved using import: docs/images/${page_name}.png"
    ls -lh "$IMAGES_DIR/${page_name}.png"

    # Close terminal window
    xdotool windowkill "$WID" || kill $TERM_PID || true
else
    echo "Error: Failed to find window for $page_name"
fi

# Kill server
kill $SERVER_PID || true
wait $SERVER_PID 2>/dev/null || true
SERVER_PID=""

echo "Test complete!"
