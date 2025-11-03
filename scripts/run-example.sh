#!/usr/bin/env bash
set -euo pipefail

# Find all demo.sh scripts in examples directory
mapfile -t scripts < <(find examples -type f -name "demo.sh" | sort)

if [ ${#scripts[@]} -eq 0 ]; then
    echo "No example scripts found in examples/"
    exit 1
fi

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
dir=$(dirname "$selected")

echo ""
echo "Running: $selected"
echo "=========================================="
echo ""

cd "$dir" && bash demo.sh
