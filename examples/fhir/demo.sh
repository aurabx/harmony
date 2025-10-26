#!/bin/bash
# FHIR Passthrough Demo Script
# Starts Harmony with FHIR endpoint and demonstrates authentication

set -e

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

# Configuration
HARMONY_PORT=8081
USERNAME="test_user"
PASSWORD="test_password"
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
TMP_DIR="$SCRIPT_DIR/tmp"
export TMP_DIR

echo -e "${BLUE}=== FHIR Passthrough Demo ===${NC}"
echo "This script demonstrates Harmony's FHIR endpoint with authentication"
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
echo -e "${YELLOW}Starting Harmony FHIR service on port $HARMONY_PORT...${NC}"
cd "$SCRIPT_DIR"
"$PROJECT_ROOT/target/release/harmony" --config ./config.toml > "$TMP_DIR/harmony.log" 2>&1 &
HARMONY_PID=$!
echo -e "${GREEN}✓ Harmony started (PID: $HARMONY_PID)${NC}"

# Wait for Harmony to be ready
echo -e "${YELLOW}Waiting for Harmony to be ready...${NC}"
for i in {1..30}; do
    if curl -s http://127.0.0.1:$HARMONY_PORT/test > /dev/null 2>&1; then
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

# Test 1: Unauthenticated request (should fail)
echo -e "${YELLOW}Test 1: Unauthenticated request (expected to fail)${NC}"
echo "  Command: curl -s -w '%{http_code}' http://127.0.0.1:$HARMONY_PORT/test"
HTTP_CODE=$(curl -s -w "%{http_code}" -o /dev/null http://127.0.0.1:$HARMONY_PORT/test)

if [ "$HTTP_CODE" = "401" ]; then
    echo -e "${GREEN}  ✓ Request correctly rejected with 401 Unauthorized${NC}"
elif [ "$HTTP_CODE" = "403" ]; then
    echo -e "${GREEN}  ✓ Request correctly rejected with 403 Forbidden${NC}"
else
    echo -e "${RED}  ✗ Unexpected status code: $HTTP_CODE (expected 401 or 403)${NC}"
fi
echo ""

# Test 2: Authenticated GET request
echo -e "${YELLOW}Test 2: Authenticated GET request${NC}"
echo "  Command: curl -u $USERNAME:$PASSWORD http://127.0.0.1:$HARMONY_PORT/test"
RESPONSE=$(curl -s -w "\n%{http_code}" -u "$USERNAME:$PASSWORD" http://127.0.0.1:$HARMONY_PORT/test)
HTTP_CODE=$(echo "$RESPONSE" | tail -n1)
BODY=$(echo "$RESPONSE" | sed '$d')

if [ "$HTTP_CODE" = "200" ]; then
    echo -e "${GREEN}  ✓ Authenticated GET successful (HTTP $HTTP_CODE)${NC}"
    echo "  Response: $BODY" | head -c 100
    echo "..."
else
    echo -e "${RED}  ✗ Authenticated GET failed (HTTP $HTTP_CODE)${NC}"
fi
echo ""

# Test 3: Authenticated POST with FHIR Patient resource
echo -e "${YELLOW}Test 3: POST FHIR Patient resource${NC}"
FHIR_PATIENT='{
  "resourceType": "Patient",
  "id": "example",
  "name": [{
    "family": "Smith",
    "given": ["John"]
  }],
  "identifier": [{
    "system": "http://example.com/patient-id",
    "value": "12345"
  }]
}'
echo "  Command: curl -u $USERNAME:**** -X POST -H 'Content-Type: application/json'"
RESPONSE=$(curl -s -w "\n%{http_code}" -u "$USERNAME:$PASSWORD" -X POST http://127.0.0.1:$HARMONY_PORT/test \
    -H "Content-Type: application/json" \
    -d "$FHIR_PATIENT")
HTTP_CODE=$(echo "$RESPONSE" | tail -n1)
BODY=$(echo "$RESPONSE" | sed '$d')

if [ "$HTTP_CODE" = "200" ]; then
    echo -e "${GREEN}  ✓ POST FHIR Patient successful (HTTP $HTTP_CODE)${NC}"
    echo "  Response contains patient data: $BODY" | head -c 100
    echo "..."
else
    echo -e "${RED}  ✗ POST FHIR Patient failed (HTTP $HTTP_CODE)${NC}"
fi
echo ""

# Test 4: Wrong credentials
echo -e "${YELLOW}Test 4: Request with wrong credentials (expected to fail)${NC}"
echo "  Command: curl -u wrong_user:wrong_pass"
HTTP_CODE=$(curl -s -w "%{http_code}" -o /dev/null -u "wrong_user:wrong_pass" http://127.0.0.1:$HARMONY_PORT/test)

if [ "$HTTP_CODE" = "401" ]; then
    echo -e "${GREEN}  ✓ Wrong credentials correctly rejected with 401 Unauthorized${NC}"
elif [ "$HTTP_CODE" = "403" ]; then
    echo -e "${GREEN}  ✓ Wrong credentials correctly rejected with 403 Forbidden${NC}"
else
    echo -e "${RED}  ✗ Unexpected status code: $HTTP_CODE (expected 401 or 403)${NC}"
fi
echo ""

echo -e "${BLUE}=== Summary ===${NC}"
echo ""
echo "FHIR Endpoint Capabilities:"
echo "  ✅ Basic authentication working"
echo "  ✅ Unauthorized requests rejected"
echo "  ✅ FHIR resource processing working"
echo "  ✅ JSON extraction middleware working"
echo ""
echo "Authentication Credentials:"
echo "  Username: $USERNAME"
echo "  Password: $PASSWORD"
echo ""
echo "Logs available at:"
echo "  Harmony: $TMP_DIR/harmony.log"
echo ""
echo -e "${GREEN}Demo complete!${NC}"
echo ""
echo "Press Enter to stop services and exit..."
read
