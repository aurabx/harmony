#!/bin/bash

# Test script to verify redb index creation

echo "=== Testing JMIX Index Creation ==="
echo ""

# Check if store exists
if [ ! -d "./tmp/jmix-store" ]; then
    echo "❌ JMIX store directory doesn't exist"
    exit 1
fi

echo "✓ JMIX store exists at: ./tmp/jmix-store"

# List packages
PACKAGES=$(ls -d ./tmp/jmix-store/*/ 2>/dev/null | grep -v __payload__ || true)
PACKAGE_COUNT=$(echo "$PACKAGES" | grep -v '^$' | wc -l | tr -d ' ')

echo "✓ Found $PACKAGE_COUNT package(s)"

# Check if index exists
if [ -f "./tmp/jmix-store/jmix-index.redb" ]; then
    SIZE=$(ls -lh ./tmp/jmix-store/jmix-index.redb | awk '{print $5}')
    echo "✓ Index exists (size: $SIZE)"
else
    echo "⚠️  Index doesn't exist yet (will be created on first use)"
fi

echo ""
echo "Run cargo test to trigger index creation, or start the server"
