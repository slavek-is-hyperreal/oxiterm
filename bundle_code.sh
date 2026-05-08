#!/bin/bash

# bundle_code.sh - OxiTerm Code Bundler
# Generates a single output containing directory structure and all relevant source code.

OUTPUT_FILE="project_bundle.txt"

{
    echo "================================================================================"
    echo "PROJECT: OxiTerm"
    echo "GENERATED: $(date)"
    echo "================================================================================"
    echo ""
    echo "=== DIRECTORY STRUCTURE (Up to Level 5) ==="
    echo "--------------------------------------------------------------------------------"
    tree -L 5 -I "target|node_modules|.git|.cargo"
    echo ""

    echo "=== RUST SOURCE FILES (.rs) ==="
    echo "--------------------------------------------------------------------------------"
    find . -name "*.rs" -not -path "./target/*" -not -path "./.git/*" -not -path "./.cargo/*" -print0 | sort -z | while IFS= read -r -d '' file; do
        echo ""
        echo "--- FILE: $file ---"
        cat "$file"
        echo ""
        echo "--- END OF FILE: $file ---"
    done

    echo ""
    echo "=== IMPORTANT PROJECT FILES (Cargo.toml, .thtml, .md) ==="
    echo "--------------------------------------------------------------------------------"
    
    # Define important non-rust files and use while read to handle spaces
    find . \( -name "*.toml" -o -name "*.thtml" -o -name "*.md" \) \
        -not -path "./target/*" \
        -not -path "./.git/*" \
        -not -path "./.cargo/*" \
        -not -path "*/node_modules/*" -print0 | sort -z | while IFS= read -r -d '' file; do
        echo ""
        echo "--- FILE: $file ---"
        cat "$file"
        echo ""
        echo "--- END OF FILE: $file ---"
    done

} > "$OUTPUT_FILE"

echo "Success: Project bundle created at $OUTPUT_FILE"
