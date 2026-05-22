#!/bin/bash
# scripts/test_terminals.sh
# Automation script for testing OxiTerm rendering on different terminal emulators on Linux Mint (X11).

set -e

# Terminals to test
TERMINALS=("xterm" "alacritty" "kitty" "gnome-terminal")

# Ensure required packages are present
echo "Checking required system tools..."
for tool in xdotool scrot; do
    if ! command -v $tool &> /dev/null; then
        echo "Error: $tool is not installed. Please install it using: sudo apt install $tool"
        exit 1
    fi
done

# Clean and recreate screenshots directory
echo "Cleaning old screenshots..."
rm -rf test_screenshots
mkdir -p test_screenshots

# Build the executable once before testing to speed up startup in the loops
echo "Building oxiterm-cli..."
cargo build --bin oxiterm-cli --features oxiterm-server/web

# Cleanup trap to kill any leftover processes
cleanup() {
    echo "Cleaning up..."
    if [ -n "$SERVER_PID" ]; then
        kill $SERVER_PID 2>/dev/null || true
    fi
    for term in "${TERMINALS[@]}"; do
        pkill -f "$term -e ssh" || true
        pkill -f "$term ssh" || true
    done
}
trap cleanup EXIT

# Iterate through each terminal emulator
for term in "${TERMINALS[@]}"; do
    if ! command -v $term &> /dev/null; then
        echo "Terminal $term is not installed on this system. Skipping..."
        continue
    fi

    # Determine class name for xdotool
    case $term in
        xterm) CLASS="xterm" ;;
        alacritty) CLASS="Alacritty" ;;
        kitty) CLASS="kitty" ;;
        gnome-terminal) CLASS="gnome-terminal" ;;
    esac

    # Iterate through each THTML page
    for page_path in examples/*.thtml; do
        page_name=$(basename "$page_path" .thtml)
        echo "Testing terminal: $term | Page: $page_name..."

        # Start oxiterm-server with the specific page
        ./target/debug/oxiterm-cli serve --port 8022 --no-auth "$page_path" > server.log 2>&1 &
        SERVER_PID=$!

        # Wait for server to bind
        sleep 1.5

        # Get active window IDs of this class before spawning
        BEFORE_WIDS=$(xdotool search --class "$CLASS" 2>/dev/null || true)

        # Spawn terminal connecting to localhost:8022
        case $term in
            xterm)
                xterm -geometry 100x36 -title "OxiTerm-Test-xterm" -e "ssh -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null -p 8022 localhost" &
                TERM_PID=$!
                ;;
            alacritty)
                alacritty -o "window.dimensions.columns=100" -o "window.dimensions.lines=36" --title "OxiTerm-Test-alacritty" -e ssh -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null -p 8022 localhost &
                TERM_PID=$!
                ;;
            kitty)
                kitty -o remember_window_size=no -o initial_window_width=100c -o initial_window_height=36c --title "OxiTerm-Test-kitty" ssh -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null -p 8022 localhost &
                TERM_PID=$!
                ;;
            gnome-terminal)
                gnome-terminal --geometry=100x36 --title="OxiTerm-Test-gnome-terminal" -- ssh -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null -p 8022 localhost &
                TERM_PID=$!
                ;;
        esac

        # Wait for window to appear
        sleep 2.5

        # Get active window IDs of this class after spawning
        AFTER_WIDS=$(xdotool search --class "$CLASS" 2>/dev/null || true)

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

        if [ -z "$WID" ]; then
            echo "Failed to find window for $term"
            kill $SERVER_PID || true
            wait $SERVER_PID 2>/dev/null || true
            SERVER_PID=""
            continue
        fi

        # Activate/focus window and wait 2s for rendering to settle
        xdotool windowactivate "$WID"
        sleep 2

        # Capture screenshots
        if [ "$page_name" = "vector_demo" ] || [ "$page_name" = "video_demo" ]; then
            echo "Capturing 5 animated frames for $term ($page_name)..."
            for frame in {1..5}; do
                scrot -u "test_screenshots/${term}_${page_name}_frame_${frame}.png"
                echo "  Saved test_screenshots/${term}_${page_name}_frame_${frame}.png"
                sleep 0.2
            done
        else
            echo "Capturing single frame for $term ($page_name)..."
            scrot -u "test_screenshots/${term}_${page_name}.png"
            echo "  Saved test_screenshots/${term}_${page_name}.png"
        fi

        # Close terminal window
        xdotool windowkill "$WID" || kill $TERM_PID || true
        sleep 1

        # Stop server
        kill $SERVER_PID || true
        wait $SERVER_PID 2>/dev/null || true
        SERVER_PID=""
    done
done

echo "All tests completed! Screenshots are in test_screenshots/ directory."
