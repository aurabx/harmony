#!/usr/bin/env bash
set -euo pipefail

# Build list of runnable examples:
# - Prefer examples with demo.sh
# - Fallback to examples that have a config.toml (run Harmony with that config)
mapfile -t example_dirs < <(find examples -mindepth 1 -maxdepth 1 -type d | sort)

scripts=()
for d in "${example_dirs[@]}"; do
  if [ -f "$d/demo.sh" ]; then
    scripts+=("$d/demo.sh")
  elif [ -f "$d/config.toml" ]; then
    scripts+=("$d/config.toml")
  fi
done

if [ ${#scripts[@]} -eq 0 ]; then
    echo "No runnable examples found in examples/"
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
        dir=$(dirname "${scripts[$i]}")
        name=$(basename "$dir")
        suffix=""
        if [ "$(basename "${scripts[$i]}")" = "config.toml" ]; then
          suffix=" (auto-run)"
        fi
        printf "%2d) %s%s\n" $((i+1)) "$name" "$suffix"
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
base=$(basename "$selected")

echo ""
echo "Running: $selected"
echo "=========================================="
echo ""

PROJECT_ROOT="$(cd "$(dirname "$0")/.." && pwd)"

if [ "$base" = "demo.sh" ]; then
  cd "$dir" && bash demo.sh
else
  # Auto-run using Harmony with the example's config.toml
  cd "$dir"
  if [ ! -x "$PROJECT_ROOT/target/release/harmony" ]; then
    echo "Building Harmony (release) ..."
    (cd "$PROJECT_ROOT" && cargo build --release)
  fi
  echo "Starting Harmony with $dir/config.toml (Ctrl+C to stop)"
  "$PROJECT_ROOT/target/release/harmony" --config ./config.toml
fi
