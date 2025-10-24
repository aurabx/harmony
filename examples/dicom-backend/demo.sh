#!/bin/bash
# DICOM Backend Demo Script
# Demonstrates HTTP to DICOM operations

set -e

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

# Configuration
HARMONY_PORT=8085
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
TMP_DIR="$SCRIPT_DIR/tmp"
export TMP_DIR

echo -e "${BLUE}=== DICOM Backend Demo ===${NC}"
echo "This script demonstrates triggering DICOM operations via HTTP"
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
echo -e "${YELLOW}Note: This example requires Orthanc PACS at localhost:4242${NC}"
echo -e "${YELLOW}      If Orthanc is not available, tests will show connection errors${NC}"
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
echo -e "${YELLOW}Starting Harmony DICOM backend service on port $HARMONY_PORT...${NC}"
cd "$SCRIPT_DIR"
"$PROJECT_ROOT/target/release/harmony" --config ./config.toml > "$TMP_DIR/harmony.log" 2>&1 &
HARMONY_PID=$!
echo -e "${GREEN}✓ Harmony started (PID: $HARMONY_PID)${NC}"

# Wait for Harmony to be ready
echo -e "${YELLOW}Waiting for Harmony to be ready...${NC}"
for i in {1..30}; do
    if curl -s http://127.0.0.1:$HARMONY_PORT/trigger-dicom > /dev/null 2>&1; then
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

# Test 1: C-FIND operation
echo -e "${YELLOW}Test 1: Trigger C-FIND operation${NC}"
CFIND_REQUEST='{
  "operation": "C-FIND",
  "level": "STUDY",
  "query": {
    "PatientID": "*",
    "StudyInstanceUID": ""
  }
}'
echo "  Command: curl -X POST http://127.0.0.1:$HARMONY_PORT/trigger-dicom"
echo "  Payload: C-FIND request for all studies"
RESPONSE=$(curl -s -w "\n%{http_code}" -X POST http://127.0.0.1:$HARMONY_PORT/trigger-dicom \
    -H "Content-Type: application/json" \
    -d "$CFIND_REQUEST")
HTTP_CODE=$(echo "$RESPONSE" | tail -n1)
BODY=$(echo "$RESPONSE" | sed '$d')

if [ "$HTTP_CODE" = "200" ]; then
    echo -e "${GREEN}  ✓ C-FIND operation successful (HTTP $HTTP_CODE)${NC}"
    echo "  Response: $BODY" | head -c 200
    echo "..."
else
    echo -e "${YELLOW}  ⚠ C-FIND operation returned HTTP $HTTP_CODE${NC}"
    echo "  This may indicate Orthanc PACS is not available"
    echo "  Response: $BODY"
fi
echo ""

# Test 2: C-FIND with specific patient
echo -e "${YELLOW}Test 2: C-FIND for specific patient${NC}"
CFIND_PATIENT='{
  "operation": "C-FIND",
  "level": "PATIENT",
  "query": {
    "PatientID": "12345",
    "PatientName": ""
  }
}'
echo "  Command: curl -X POST (C-FIND for PatientID=12345)"
RESPONSE=$(curl -s -w "\n%{http_code}" -X POST http://127.0.0.1:$HARMONY_PORT/trigger-dicom \
    -H "Content-Type: application/json" \
    -d "$CFIND_PATIENT")
HTTP_CODE=$(echo "$RESPONSE" | tail -n1)

if [ "$HTTP_CODE" = "200" ]; then
    echo -e "${GREEN}  ✓ Patient-level C-FIND successful (HTTP $HTTP_CODE)${NC}"
else
    echo -e "${YELLOW}  ⚠ Patient-level C-FIND returned HTTP $HTTP_CODE${NC}"
fi
echo ""

# Test 3: C-MOVE operation
echo -e "${YELLOW}Test 3: Trigger C-MOVE operation${NC}"
CMOVE_REQUEST='{
  "operation": "C-MOVE",
  "destination": "HARMONY_SCU",
  "query": {
    "StudyInstanceUID": "1.2.3.4.5.6"
  }
}'
echo "  Command: curl -X POST (C-MOVE request)"
RESPONSE=$(curl -s -w "\n%{http_code}" -X POST http://127.0.0.1:$HARMONY_PORT/trigger-dicom \
    -H "Content-Type: application/json" \
    -d "$CMOVE_REQUEST")
HTTP_CODE=$(echo "$RESPONSE" | tail -n1)

if [ "$HTTP_CODE" = "200" ] || [ "$HTTP_CODE" = "202" ]; then
    echo -e "${GREEN}  ✓ C-MOVE operation accepted (HTTP $HTTP_CODE)${NC}"
else
    echo -e "${YELLOW}  ⚠ C-MOVE operation returned HTTP $HTTP_CODE${NC}"
fi
echo ""

echo -e "${BLUE}=== Summary ===${NC}"
echo ""
echo "DICOM Backend Capabilities:"
echo "  ✅ HTTP to DICOM protocol translation"
echo "  ✅ C-FIND operations (PATIENT, STUDY, SERIES, IMAGE levels)"
echo "  ✅ C-MOVE operations"
echo "  ✅ RESTful API for DICOM operations"
echo ""
echo "Supported Operations:"
echo "  C-ECHO:  Verify connection"
echo "  C-FIND:  Query for studies, series, images"
echo "  C-MOVE:  Retrieve studies from PACS"
echo "  C-STORE: Send DICOM objects to PACS"
echo ""
echo "Configuration:"
echo "  DICOM Backend: Orthanc (localhost:4242)"
echo "  Local AE:      HARMONY_SCU"
echo "  Remote AE:     ORTHANC"
echo ""
echo "To test with real DICOM PACS:"
echo "  1. Start Orthanc: docker run -p 4242:4242 -p 8042:8042 orthancteam/orthanc"
echo "  2. Upload DICOM studies to Orthanc"
echo "  3. Trigger operations via HTTP POST to /trigger-dicom"
echo ""
echo "Logs available at:"
echo "  Harmony: $TMP_DIR/harmony.log"
echo ""
echo -e "${GREEN}Demo complete!${NC}"
echo ""
echo "Press Enter to stop services and exit..."
read
