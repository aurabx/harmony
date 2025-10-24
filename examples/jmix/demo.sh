#!/bin/bash
# JMIX Demo Script
# Demonstrates JMIX envelope creation and retrieval

set -e

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

# Configuration
HARMONY_PORT=8084
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
TMP_DIR="$SCRIPT_DIR/tmp"
export TMP_DIR

echo -e "${BLUE}=== JMIX Demo ===${NC}"
echo "This script demonstrates Harmony's JMIX envelope capabilities"
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
echo -e "${YELLOW}Note: This example requires Orthanc PACS for full functionality${NC}"
echo -e "${YELLOW}      If Orthanc is not available, some operations may not work${NC}"
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
echo -e "${YELLOW}Starting Harmony JMIX service on port $HARMONY_PORT...${NC}"
cd "$SCRIPT_DIR"
"$PROJECT_ROOT/target/release/harmony" --config ./config.toml > "$TMP_DIR/harmony.log" 2>&1 &
HARMONY_PID=$!
echo -e "${GREEN}✓ Harmony started (PID: $HARMONY_PID)${NC}"

# Wait for Harmony to be ready
echo -e "${YELLOW}Waiting for Harmony to be ready...${NC}"
for i in {1..30}; do
    if curl -s http://127.0.0.1:$HARMONY_PORT/jmix/api/jmix > /dev/null 2>&1; then
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

# Test 1: Create JMIX envelope request
echo -e "${YELLOW}Test 1: Request JMIX envelope for study${NC}"
STUDY_UID="1.3.6.1.4.1.5962.99.1.939772310.1977867020.1426868947350.4.0"
echo "  Command: curl 'http://127.0.0.1:$HARMONY_PORT/jmix/api/jmix?studyInstanceUid=$STUDY_UID'"

# Save response to temp file to handle binary zip data
RESP_FILE="$TMP_DIR/jmix_response.zip"
HTTP_CODE=$(curl -s -w "%{http_code}" -o "$RESP_FILE" "http://127.0.0.1:$HARMONY_PORT/jmix/api/jmix?studyInstanceUid=$STUDY_UID")

if [ "$HTTP_CODE" = "200" ] || [ "$HTTP_CODE" = "201" ] || [ "$HTTP_CODE" = "202" ]; then
    echo -e "${GREEN}  ✓ JMIX envelope request successful (HTTP $HTTP_CODE)${NC}"
    
    # Check if response is a zip file
    if file "$RESP_FILE" | grep -q "Zip archive"; then
        FILE_SIZE=$(stat -f%z "$RESP_FILE" 2>/dev/null || stat -c%s "$RESP_FILE" 2>/dev/null)
        echo -e "${GREEN}  ✓ Received JMIX envelope as zip file (${FILE_SIZE} bytes)${NC}"
        
        # Try to extract envelope ID from zip manifest.json
        if command -v unzip &> /dev/null; then
            MANIFEST_CONTENT=$(unzip -p "$RESP_FILE" manifest.json 2>/dev/null || echo "")
            if [ ! -z "$MANIFEST_CONTENT" ]; then
                if command -v jq &> /dev/null; then
                    ENVELOPE_ID=$(echo "$MANIFEST_CONTENT" | jq -r '.id // empty' 2>/dev/null || echo "")
                else
                    ENVELOPE_ID=$(echo "$MANIFEST_CONTENT" | grep -o '"id"[[:space:]]*:[[:space:]]*"[^"]*"' | head -1 | cut -d'"' -f4 || echo "")
                fi
                if [ ! -z "$ENVELOPE_ID" ]; then
                    echo -e "${GREEN}  ✓ Envelope ID from manifest: $ENVELOPE_ID${NC}"
                fi
            fi
        fi
    else
        echo "failed"
#        # Not a zip, might be JSON response
#        BODY=$(cat "$RESP_FILE")
#        echo "  Response: $BODY"
#
#        # Try to extract envelope ID from JSON response
#        if command -v jq &> /dev/null; then
#            ENVELOPE_ID=$(echo "$BODY" | jq -r '.jmixEnvelopes[0].id // empty' 2>/dev/null || echo "")
#        else
#            # Fallback to grep if jq not available
#            ENVELOPE_ID=$(echo "$BODY" | grep -o '"id"[[:space:]]*:[[:space:]]*"[^"]*"' | head -1 | cut -d'"' -f4 || echo "")
#        fi
#
#        if [ ! -z "$ENVELOPE_ID" ]; then
#            echo -e "${GREEN}  ✓ Envelope ID: $ENVELOPE_ID${NC}"
#        fi
    fi
else
    echo -e "${YELLOW}  ⚠ JMIX envelope request returned HTTP $HTTP_CODE${NC}"
    echo "  This may indicate Orthanc PACS is not available"
    BODY=$(cat "$RESP_FILE")
    echo "  Response: $BODY"
fi
echo ""

# Test 2: Request manifest for envelope
echo -e "${YELLOW}Test 2: Request manifest for JMIX envelope${NC}"
# Use the envelope ID from Test 1 if available, otherwise use a test ID
TEST_ID="${ENVELOPE_ID:-test-envelope-id}"
echo "  Command: curl http://127.0.0.1:$HARMONY_PORT/jmix/api/jmix/$TEST_ID/manifest"
RESPONSE=$(curl -s -w "\n%{http_code}" "http://127.0.0.1:$HARMONY_PORT/jmix/api/jmix/$TEST_ID/manifest")
HTTP_CODE=$(echo "$RESPONSE" | tail -n1)
BODY=$(echo "$RESPONSE" | sed '$d')

if [ "$HTTP_CODE" = "200" ]; then
    echo -e "${GREEN}  ✓ Manifest request successful (HTTP $HTTP_CODE)${NC}"
    # Pretty print JSON if jq is available
    if command -v jq &> /dev/null; then
        echo "  Manifest content:"
        echo "$BODY" | jq '.' | head -20
    else
        echo "  Response: $BODY" | head -c 500
    fi
elif [ "$HTTP_CODE" = "404" ]; then
    echo -e "${YELLOW}  ⚠ Manifest not found (HTTP $HTTP_CODE)${NC}"
    echo "  This is expected if the envelope ID '$TEST_ID' does not exist"
else
    echo -e "${YELLOW}  ⚠ Manifest request returned HTTP $HTTP_CODE${NC}"
    echo "  Response: $BODY"
fi
echo ""

echo -e "${BLUE}=== Summary ===${NC}"
echo ""
echo "JMIX Endpoint Capabilities:"
echo "  ✅ JMIX envelope creation API"
echo "  ✅ Study-based envelope requests"
echo "  ✅ Performance optimization (skip_hashing enabled)"
echo "  ✅ File listing enabled"
echo ""
echo "Configuration:"
echo "  DICOM Backend: Orthanc (127.0.0.1:4242)"
echo "  Local AE:      HARMONY_SCU"
echo "  Remote AE:     ORTHANC"
echo "  Performance:   skip_hashing=true, skip_listing=false"
echo ""
echo "To test with real DICOM data:"
echo "  1. Start Orthanc: docker run -p 4242:4242 -p 8042:8042 orthancteam/orthanc"
echo "  2. Upload DICOM studies to Orthanc"
echo "  3. Request envelope: curl 'http://127.0.0.1:$HARMONY_PORT/jmix/api/jmix?studyInstanceUid=<UID>'"
echo ""
echo "Logs available at:"
echo "  Harmony: $TMP_DIR/harmony.log"
echo ""
echo -e "${GREEN}Demo complete!${NC}"
echo ""
echo "Press Enter to stop services and exit..."
read
