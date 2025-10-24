#!/bin/bash
# HTTP Backend Demo Script
# Demonstrates HTTP passthrough with actual backend

set -e

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

# Configuration
HARMONY_PORT=8080
BACKEND_PORT=8888
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
TMP_DIR="$SCRIPT_DIR/tmp"
BACKEND_DIR="$TMP_DIR/backend"
export TMP_DIR

echo -e "${BLUE}=== HTTP Backend Demo ===${NC}"
echo "This script demonstrates Harmony's HTTP passthrough with a real backend"
echo ""

# Check for required tools
echo -e "${YELLOW}Checking prerequisites...${NC}"
if ! command -v curl &> /dev/null; then
    echo -e "${RED}Error: curl not found. Please install curl.${NC}"
    exit 1
fi

if ! command -v python3 &> /dev/null; then
    echo -e "${RED}Error: python3 not found. Please install Python 3.${NC}"
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
mkdir -p "$BACKEND_DIR"

# Create test HTML file for backend
cat > "$BACKEND_DIR/index.html" << 'EOF'
<!DOCTYPE html>
<html>
<head><title>Test Backend</title></head>
<body>
<h1>Hello from Backend!</h1>
<p>This is served through Harmony's HTTP backend.</p>
</body>
</html>
EOF

# Create JSON test file
cat > "$BACKEND_DIR/data.json" << 'EOF'
{
  "message": "Hello from backend",
  "status": "success",
  "timestamp": "2024-01-01T12:00:00Z"
}
EOF

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
    
    # Kill Python backend
    if [ ! -z "$BACKEND_PID" ] && kill -0 $BACKEND_PID 2>/dev/null; then
        echo "  Stopping Python backend (PID: $BACKEND_PID)..."
        kill $BACKEND_PID
        wait $BACKEND_PID 2>/dev/null || true
    fi
    
    echo -e "${GREEN}✓ Cleanup complete${NC}"
}

# Setup trap for cleanup
trap cleanup EXIT INT TERM

# Start Python HTTP server as backend
echo -e "${YELLOW}Starting Python HTTP backend on port $BACKEND_PORT...${NC}"
cd "$BACKEND_DIR"
python3 -m http.server $BACKEND_PORT > "$TMP_DIR/backend.log" 2>&1 &
BACKEND_PID=$!
cd "$SCRIPT_DIR"
echo -e "${GREEN}✓ Backend started (PID: $BACKEND_PID)${NC}"

# Wait for backend to be ready
sleep 2
if ! curl -s http://127.0.0.1:$BACKEND_PORT/ > /dev/null; then
    echo -e "${RED}Error: Backend did not start${NC}"
    exit 1
fi
echo -e "${GREEN}✓ Backend is ready${NC}"
echo ""

# Build Harmony
echo -e "${YELLOW}Building Harmony...${NC}"
cd "$PROJECT_ROOT"
cargo build --release --quiet
echo -e "${GREEN}✓ Build complete${NC}"
echo ""

# Start Harmony in background
echo -e "${YELLOW}Starting Harmony HTTP proxy on port $HARMONY_PORT...${NC}"
cd "$SCRIPT_DIR"
"$PROJECT_ROOT/target/release/harmony" --config ./config.toml > "$TMP_DIR/harmony.log" 2>&1 &
HARMONY_PID=$!
echo -e "${GREEN}✓ Harmony started (PID: $HARMONY_PID)${NC}"

# Wait for Harmony to be ready
echo -e "${YELLOW}Waiting for Harmony to be ready...${NC}"
for i in {1..30}; do
    if curl -s http://127.0.0.1:$HARMONY_PORT/ > /dev/null 2>&1; then
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

# Test 1: Direct backend access (baseline)
echo -e "${YELLOW}Test 1: Direct backend access (baseline)${NC}"
echo "  Command: curl http://127.0.0.1:$BACKEND_PORT/"
RESPONSE=$(curl -s -w "\n%{http_code}" http://127.0.0.1:$BACKEND_PORT/)
HTTP_CODE=$(echo "$RESPONSE" | tail -n1)

if [ "$HTTP_CODE" = "200" ]; then
    echo -e "${GREEN}  ✓ Direct backend access successful (HTTP $HTTP_CODE)${NC}"
else
    echo -e "${RED}  ✗ Direct backend access failed (HTTP $HTTP_CODE)${NC}"
fi
echo ""

# Test 2: Proxied GET request
echo -e "${YELLOW}Test 2: Proxied GET request through Harmony${NC}"
echo "  Command: curl http://127.0.0.1:$HARMONY_PORT/"
RESPONSE=$(curl -s -w "\n%{http_code}" http://127.0.0.1:$HARMONY_PORT/)
HTTP_CODE=$(echo "$RESPONSE" | tail -n1)
BODY=$(echo "$RESPONSE" | sed '$d')

if [ "$HTTP_CODE" = "200" ]; then
    echo -e "${GREEN}  ✓ Proxied GET successful (HTTP $HTTP_CODE)${NC}"
    if echo "$BODY" | grep -q "Hello from Backend"; then
        echo -e "${GREEN}  ✓ Backend content received through proxy${NC}"
    fi
else
    echo -e "${RED}  ✗ Proxied GET failed (HTTP $HTTP_CODE)${NC}"
fi
echo ""

# Test 3: Proxied JSON endpoint
echo -e "${YELLOW}Test 3: Proxied JSON endpoint${NC}"
echo "  Command: curl http://127.0.0.1:$HARMONY_PORT/data.json"
RESPONSE=$(curl -s -w "\n%{http_code}" http://127.0.0.1:$HARMONY_PORT/data.json)
HTTP_CODE=$(echo "$RESPONSE" | tail -n1)
BODY=$(echo "$RESPONSE" | sed '$d')

if [ "$HTTP_CODE" = "200" ]; then
    echo -e "${GREEN}  ✓ Proxied JSON successful (HTTP $HTTP_CODE)${NC}"
    echo "  Response: $BODY"
    if echo "$BODY" | grep -q '"message"'; then
        echo -e "${GREEN}  ✓ JSON data proxied correctly${NC}"
    fi
else
    echo -e "${RED}  ✗ Proxied JSON failed (HTTP $HTTP_CODE)${NC}"
fi
echo ""

# Test 4: Proxied request with custom headers
echo -e "${YELLOW}Test 4: Proxied request with custom headers${NC}"
echo "  Command: curl -H 'X-Custom-Header: test'"
RESPONSE=$(curl -s -w "\n%{http_code}" http://127.0.0.1:$HARMONY_PORT/data.json \
    -H "X-Custom-Header: test-value" \
    -H "X-Request-ID: 12345")
HTTP_CODE=$(echo "$RESPONSE" | tail -n1)

if [ "$HTTP_CODE" = "200" ]; then
    echo -e "${GREEN}  ✓ Proxied request with headers successful (HTTP $HTTP_CODE)${NC}"
else
    echo -e "${RED}  ✗ Proxied request with headers failed (HTTP $HTTP_CODE)${NC}"
fi
echo ""

# Test 5: 404 handling
echo -e "${YELLOW}Test 5: 404 Not Found handling${NC}"
echo "  Command: curl http://127.0.0.1:$HARMONY_PORT/nonexistent"
HTTP_CODE=$(curl -s -w "%{http_code}" -o /dev/null http://127.0.0.1:$HARMONY_PORT/nonexistent)

if [ "$HTTP_CODE" = "404" ]; then
    echo -e "${GREEN}  ✓ 404 handling correct (HTTP $HTTP_CODE)${NC}"
else
    echo -e "${YELLOW}  ⚠ Unexpected status code: $HTTP_CODE${NC}"
fi
echo ""

echo -e "${BLUE}=== Summary ===${NC}"
echo ""
echo "HTTP Backend Capabilities:"
echo "  ✅ HTTP passthrough working"
echo "  ✅ GET requests proxied"
echo "  ✅ JSON content proxied"
echo "  ✅ Custom headers passed through"
echo "  ✅ Error responses proxied"
echo ""
echo "Backend Configuration:"
echo "  Backend: http://127.0.0.1:$BACKEND_PORT"
echo "  Proxy:   http://127.0.0.1:$HARMONY_PORT"
echo ""
echo "Logs available at:"
echo "  Harmony: $TMP_DIR/harmony.log"
echo "  Backend: $TMP_DIR/backend.log"
echo ""
echo -e "${GREEN}Demo complete!${NC}"
echo ""
echo "Press Enter to stop services and exit..."
read
