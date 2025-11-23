#!/usr/bin/env bash
set -euo pipefail

# Find all demo.sh scripts in examples directory
mapfile -t scripts < <(find examples -type f -name "demo.sh" | sort)

if [ ${#scripts[@]} -eq 0 ]; then
    echo "No example scripts found in examples/"
    exit 1
fi

# Check if an argument was provided
if [ $# -eq 1 ]; then
    target="$1"
    # Try to find matching example by name
    for i in "${!scripts[@]}"; do
        dir=$(dirname "${scripts[$i]}")
        name=$(basename "$dir")
        if [ "$name" == "$target" ]; then
            selected="${scripts[$i]}"
            echo "Selected example: $name"
            break
        fi
    done

    if [ -z "${selected:-}" ]; then
        # Try to interpret as a number
        if [[ "$target" =~ ^[0-9]+$ ]] && [ "$target" -ge 1 ] && [ "$target" -le ${#scripts[@]} ]; then
            selected="${scripts[$((target-1))]}"
        else
            echo "Error: Example '$target' not found."
            echo "Available examples:"
            for i in "${!scripts[@]}"; do
                dir=$(dirname "${scripts[$i]}")
                name=$(basename "$dir")
                echo "  $name"
            done
            exit 1
        fi
    fi
else
    # Interactive mode
    echo "Available Examples:"
    echo "==================="
    for i in "${!scripts[@]}"; do
        # Extract directory name for display
        dir=$(dirname "${scripts[$i]}")
        name=$(basename "$dir")
        printf "%2d) %s\n" $((i+1)) "$name"
    done
    echo ""
    echo " q) Quit"
    echo ""

    read -rp "Select example to run: " choice

    if [[ "$choice" == "q" || "$choice" == "Q" ]]; then
        echo "Exiting"
        exit 0
    fi

    if ! [[ "$choice" =~ ^[0-9]+$ ]] || [ "$choice" -lt 1 ] || [ "$choice" -gt ${#scripts[@]} ]; then
        echo "Invalid selection"
        exit 1
    fi

    selected="${scripts[$((choice-1))]}"
fi

dir=$(dirname "$selected")

echo ""
echo "Running: $selected"
echo "=========================================="
echo ""

cd "$dir" && bash demo.sh
