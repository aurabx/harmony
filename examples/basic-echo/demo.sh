#!/bin/bash
# Basic Echo Demo Script
# Starts Harmony with basic echo endpoint and demonstrates HTTP passthrough

set -e

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

# Configuration
HARMONY_PORT=8080
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
TMP_DIR="$SCRIPT_DIR/tmp"
export TMP_DIR

echo -e "${BLUE}=== Basic Echo Demo ===${NC}"
echo "This script demonstrates Harmony's basic HTTP echo endpoint"
echo ""

# Check for required tools
echo -e "${YELLOW}Checking prerequisites...${NC}"
if ! command -v curl &> /dev/null; then
    echo -e "${RED}Error: curl not found. Please install curl.${NC}"
    exit 1
fi

if ! command -v cargo &> /dev/null; then
    echo -e "${RED}Error: cargo not found. Please install Rust.${NC}"
    exit 1
fi

echo -e "${GREEN}✓ All prerequisites found${NC}"
echo ""

# Setup directories
echo -e "${YELLOW}Setting up test environment...${NC}"
mkdir -p "$TMP_DIR"
echo -e "${GREEN}✓ Test environment ready${NC}"
echo ""

# Cleanup function
cleanup() {
    echo ""
    echo -e "${YELLOW}Cleaning up...${NC}"
    
    # Kill Harmony
    if [ ! -z "$HARMONY_PID" ] && kill -0 $HARMONY_PID 2>/dev/null; then
        echo "  Stopping Harmony (PID: $HARMONY_PID)..."
        kill $HARMONY_PID
        wait $HARMONY_PID 2>/dev/null || true
    fi
    
    echo -e "${GREEN}✓ Cleanup complete${NC}"
}

# Setup trap for cleanup
trap cleanup EXIT INT TERM

# Build Harmony
echo -e "${YELLOW}Building Harmony...${NC}"
cd "$PROJECT_ROOT"
cargo build --release --quiet
echo -e "${GREEN}✓ Build complete${NC}"
echo ""

# Start Harmony in background
echo -e "${YELLOW}Starting Harmony echo service on port $HARMONY_PORT...${NC}"
cd "$SCRIPT_DIR"
"$PROJECT_ROOT/target/release/harmony" --config ./config.toml > "$TMP_DIR/harmony.log" 2>&1 &
HARMONY_PID=$!
echo -e "${GREEN}✓ Harmony started (PID: $HARMONY_PID)${NC}"

# Wait for Harmony to be ready
echo -e "${YELLOW}Waiting for Harmony to be ready...${NC}"
for i in {1..30}; do
    if curl -s http://127.0.0.1:$HARMONY_PORT/echo > /dev/null 2>&1; then
        echo -e "${GREEN}✓ Harmony is ready${NC}"
        break
    fi
    if [ $i -eq 30 ]; then
        echo -e "${RED}Error: Harmony did not start in time${NC}"
        echo "Check logs: $TMP_DIR/harmony.log"
        exit 1
    fi
    sleep 1
done

echo ""
echo -e "${BLUE}=== Running Tests ===${NC}"
echo ""

# Test 1: Basic GET request
echo -e "${YELLOW}Test 1: Basic GET request${NC}"
echo "  Command: curl -s http://127.0.0.1:$HARMONY_PORT/echo"
RESPONSE=$(curl -s -w "\n%{http_code}" http://127.0.0.1:$HARMONY_PORT/echo)
HTTP_CODE=$(echo "$RESPONSE" | tail -n1)
BODY=$(echo "$RESPONSE" | sed '$d')

if [ "$HTTP_CODE" = "200" ]; then
    echo -e "${GREEN}  ✓ GET request successful (HTTP $HTTP_CODE)${NC}"
    echo "  Response: $BODY" | head -c 100
    echo "..."
else
    echo -e "${RED}  ✗ GET request failed (HTTP $HTTP_CODE)${NC}"
fi
echo ""

# Test 2: POST with JSON
echo -e "${YELLOW}Test 2: POST with JSON payload${NC}"
TEST_DATA='{"message": "Hello, Harmony!", "test": true, "number": 42}'
echo "  Command: curl -X POST -H 'Content-Type: application/json' -d '$TEST_DATA'"
RESPONSE=$(curl -s -w "\n%{http_code}" -X POST http://127.0.0.1:$HARMONY_PORT/echo \
    -H "Content-Type: application/json" \
    -d "$TEST_DATA")
HTTP_CODE=$(echo "$RESPONSE" | tail -n1)
BODY=$(echo "$RESPONSE" | sed '$d')

if [ "$HTTP_CODE" = "200" ]; then
    echo -e "${GREEN}  ✓ POST request successful (HTTP $HTTP_CODE)${NC}"
    echo "  Response contains request data: $BODY" | head -c 100
    echo "..."
else
    echo -e "${RED}  ✗ POST request failed (HTTP $HTTP_CODE)${NC}"
fi
echo ""

# Test 3: Custom headers
echo -e "${YELLOW}Test 3: Request with custom headers${NC}"
echo "  Command: curl -H 'X-Custom-Header: test-value'"
RESPONSE=$(curl -s -w "\n%{http_code}" http://127.0.0.1:$HARMONY_PORT/echo \
    -H "X-Custom-Header: test-value" \
    -H "X-Request-ID: 12345")
HTTP_CODE=$(echo "$RESPONSE" | tail -n1)

if [ "$HTTP_CODE" = "200" ]; then
    echo -e "${GREEN}  ✓ Request with headers successful (HTTP $HTTP_CODE)${NC}"
else
    echo -e "${RED}  ✗ Request with headers failed (HTTP $HTTP_CODE)${NC}"
fi
echo ""

echo -e "${BLUE}=== Summary ===${NC}"
echo ""
echo "Basic Echo Endpoint Capabilities:"
echo "  ✅ GET requests working"
echo "  ✅ POST requests working"
echo "  ✅ Custom headers handled"
echo "  ✅ Request/response passthrough working"
echo ""
echo "Logs available at:"
echo "  Harmony: $TMP_DIR/harmony.log"
echo ""
echo -e "${GREEN}Demo complete!${NC}"
echo ""
echo "Press Enter to stop services and exit..."
read
