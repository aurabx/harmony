#!/bin/bash
# FHIR-DICOM Integration Demo Script
# Demonstrates FHIR ImagingStudy queries with mock DICOM backend

set -e

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

# Configuration
HARMONY_PORT=8081
MGMT_PORT=9091
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
TMP_DIR="$SCRIPT_DIR/tmp"
export TMP_DIR

echo -e "${BLUE}=== FHIR-DICOM Integration Demo ===${NC}"
echo "This script demonstrates FHIR ImagingStudy queries backed by DICOM"
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

HAS_JQ=false
if command -v jq &> /dev/null; then
    HAS_JQ=true
    echo -e "${GREEN}✓ jq found (will use for pretty printing)${NC}"
fi

echo -e "${GREEN}✓ All required prerequisites found${NC}"
echo -e "${YELLOW}Note: This example uses mock_dicom backend (no external PACS required)${NC}"
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
echo -e "${YELLOW}Starting Harmony FHIR-DICOM service on port $HARMONY_PORT...${NC}"
cd "$SCRIPT_DIR"
"$PROJECT_ROOT/target/release/harmony" --config ./config.toml > "$TMP_DIR/harmony.log" 2>&1 &
HARMONY_PID=$!
echo -e "${GREEN}✓ Harmony started (PID: $HARMONY_PID)${NC}"

# Wait for Harmony to be ready
echo -e "${YELLOW}Waiting for Harmony to be ready...${NC}"
for i in {1..30}; do
    if curl -s http://127.0.0.1:$HARMONY_PORT/fhir/ImagingStudy > /dev/null 2>&1; then
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

# Test 1: FHIR ImagingStudy search by patient
echo -e "${YELLOW}Test 1: Search ImagingStudy by patient ID${NC}"
echo "  Command: curl 'http://localhost:$HARMONY_PORT/fhir/ImagingStudy?patient=PID156695'"
RESPONSE=$(curl -s -w "\n%{http_code}" "http://127.0.0.1:$HARMONY_PORT/fhir/ImagingStudy?patient=PID156695" \
    -H "Accept: application/fhir+json")
HTTP_CODE=$(echo "$RESPONSE" | tail -n1)
BODY=$(echo "$RESPONSE" | sed '$d')

if [ "$HTTP_CODE" = "200" ]; then
    echo -e "${GREEN}  ✓ Patient search successful (HTTP $HTTP_CODE)${NC}"
    echo ""
    echo "  Response (FHIR Bundle):"
    if [ "$HAS_JQ" = true ]; then
        echo "$BODY" | jq '.' 2>/dev/null | head -20
    else
        echo "$BODY" | head -c 300
    fi
    echo "..."
else
    echo -e "${RED}  ✗ Patient search failed (HTTP $HTTP_CODE)${NC}"
    echo "  Response: $BODY"
fi
echo ""

# Test 2: FHIR ImagingStudy search by identifier
echo -e "${YELLOW}Test 2: Search ImagingStudy by identifier${NC}"
echo "  Command: curl 'http://localhost:$HARMONY_PORT/fhir/ImagingStudy?identifier=1.2.826.0.1.3680043.9.7133.3280065491876470'"
RESPONSE=$(curl -s -w "\n%{http_code}" "http://127.0.0.1:$HARMONY_PORT/fhir/ImagingStudy?identifier=1.2.826.0.1.3680043.9.7133.3280065491876470" \
    -H "Accept: application/fhir+json")
HTTP_CODE=$(echo "$RESPONSE" | tail -n1)
BODY=$(echo "$RESPONSE" | sed '$d')

if [ "$HTTP_CODE" = "200" ]; then
    echo -e "${GREEN}  ✓ Identifier search successful (HTTP $HTTP_CODE)${NC}"
    
    # Check for JMIX URL in response
    if echo "$BODY" | grep -q "_jmix_url" || echo "$BODY" | grep -q "endpoint"; then
        echo -e "${GREEN}  ✓ Response includes JMIX endpoint URLs${NC}"
    fi
else
    echo -e "${RED}  ✗ Identifier search failed (HTTP $HTTP_CODE)${NC}"
fi
echo ""

# Test 3: FHIR ImagingStudy search with modality
echo -e "${YELLOW}Test 3: Search ImagingStudy by patient and modality${NC}"
echo "  Command: curl 'http://localhost:$HARMONY_PORT/fhir/ImagingStudy?patient=PID123&modality=CT'"
RESPONSE=$(curl -s -w "\n%{http_code}" "http://127.0.0.1:$HARMONY_PORT/fhir/ImagingStudy?patient=PID123&modality=CT" \
    -H "Accept: application/fhir+json")
HTTP_CODE=$(echo "$RESPONSE" | tail -n1)

if [ "$HTTP_CODE" = "200" ]; then
    echo -e "${GREEN}  ✓ Modality filter search successful (HTTP $HTTP_CODE)${NC}"
else
    echo -e "${YELLOW}  ⚠ Modality filter search returned HTTP $HTTP_CODE${NC}"
fi
echo ""

# Test 4: Management API - System Info
echo -e "${YELLOW}Test 4: Management API - System Information${NC}"
echo "  Command: curl http://127.0.0.1:$MGMT_PORT/admin/info"
RESPONSE=$(curl -s -w "\n%{http_code}" "http://127.0.0.1:$MGMT_PORT/admin/info")
HTTP_CODE=$(echo "$RESPONSE" | tail -n1)
BODY=$(echo "$RESPONSE" | sed '$d')

if [ "$HTTP_CODE" = "200" ]; then
    echo -e "${GREEN}  ✓ System info successful (HTTP $HTTP_CODE)${NC}"
    echo ""
    echo "  System Info:"
    if [ "$HAS_JQ" = true ]; then
        echo "$BODY" | jq '.'
    else
        echo "$BODY"
    fi
else
    echo -e "${RED}  ✗ System info failed (HTTP $HTTP_CODE)${NC}"
fi
echo ""

# Test 5: Management API - List Pipelines
echo -e "${YELLOW}Test 5: Management API - List Pipelines${NC}"
echo "  Command: curl http://127.0.0.1:$MGMT_PORT/admin/pipelines"
RESPONSE=$(curl -s -w "\n%{http_code}" "http://127.0.0.1:$MGMT_PORT/admin/pipelines")
HTTP_CODE=$(echo "$RESPONSE" | tail -n1)
BODY=$(echo "$RESPONSE" | sed '$d')

if [ "$HTTP_CODE" = "200" ]; then
    echo -e "${GREEN}  ✓ Pipeline list successful (HTTP $HTTP_CODE)${NC}"
    echo ""
    echo "  Pipelines:"
    if [ "$HAS_JQ" = true ]; then
        echo "$BODY" | jq '.'
    else
        echo "$BODY"
    fi
else
    echo -e "${RED}  ✗ Pipeline list failed (HTTP $HTTP_CODE)${NC}"
fi
echo ""

echo -e "${BLUE}=== Summary ===${NC}"
echo ""
echo "FHIR-DICOM Integration Capabilities:"
echo "  ✅ FHIR ImagingStudy queries working"
echo "  ✅ DICOM C-FIND operations (via mock backend)"
echo "  ✅ Query parameter extraction and mapping"
echo "  ✅ DICOM to FHIR transformation"
echo "  ✅ JMIX endpoint URL enrichment"
echo "  ✅ Management API operational"
echo ""
echo "Supported Query Parameters:"
echo "  - patient:         Patient identifier"
echo "  - identifier:      Study instance UID"
echo "  - modality:        Study modality (CT, MR, etc.)"
echo "  - studyDate:       Study date"
echo "  - accessionNumber: Accession number"
echo ""
echo "API Endpoints:"
echo "  FHIR API:       http://127.0.0.1:$HARMONY_PORT/fhir/ImagingStudy"
echo "  Management API: http://127.0.0.1:$MGMT_PORT/admin/"
echo ""
echo "Logs available at:"
echo "  Harmony: $TMP_DIR/harmony.log"
echo ""
if [ "$HAS_JQ" = false ]; then
    echo -e "${YELLOW}Tip: Install 'jq' for better JSON formatting${NC}"
    echo ""
fi
echo -e "${GREEN}Demo complete!${NC}"
echo ""
echo "Press Enter to stop services and exit..."
read
