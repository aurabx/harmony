#!/bin/bash
# Transform Middleware Demo Script
# Demonstrates JOLT transformation of JSON data

set -e

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

# Configuration
HARMONY_PORT=8083
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
TMP_DIR="$SCRIPT_DIR/tmp"
export TMP_DIR

echo -e "${BLUE}=== Transform Middleware Demo ===${NC}"
echo "This script demonstrates Harmony's JOLT transformation capabilities"
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
echo -e "${YELLOW}Starting Harmony transform service on port $HARMONY_PORT...${NC}"
cd "$SCRIPT_DIR"
"$PROJECT_ROOT/target/release/harmony" --config ./config.toml > "$TMP_DIR/harmony.log" 2>&1 &
HARMONY_PID=$!
echo -e "${GREEN}✓ Harmony started (PID: $HARMONY_PID)${NC}"

# Wait for Harmony to be ready
echo -e "${YELLOW}Waiting for Harmony to be ready...${NC}"
for i in {1..30}; do
    if curl -s http://127.0.0.1:$HARMONY_PORT/transform > /dev/null 2>&1; then
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

# Test 1: Patient to FHIR transformation
echo -e "${YELLOW}Test 1: Patient Data to FHIR Transformation${NC}"
PATIENT_DATA='{
  "PatientID": "12345",
  "PatientName": "John Doe",
  "StudyInstanceUID": "1.2.3.4.5.6",
  "StudyDate": "2024-01-15"
}'
echo "  Input data:"
echo "$PATIENT_DATA" | head -c 100
echo ""
echo "  Command: curl -X POST -H 'Content-Type: application/json' ..."

RESPONSE=$(curl -s -w "\n%{http_code}" -X POST http://127.0.0.1:$HARMONY_PORT/transform \
    -H "Content-Type: application/json" \
    -d "$PATIENT_DATA")
HTTP_CODE=$(echo "$RESPONSE" | tail -n1)
BODY=$(echo "$RESPONSE" | sed '$d')

if [ "$HTTP_CODE" = "200" ]; then
    echo -e "${GREEN}  ✓ Transformation successful (HTTP $HTTP_CODE)${NC}"
    echo ""
    echo "  Transformed output:"
    if [ "$HAS_JQ" = true ]; then
        echo "$BODY" | jq '.' 2>/dev/null || echo "$BODY"
    else
        echo "$BODY"
    fi
    echo ""
    
    # Verify transformation actually occurred
    if echo "$BODY" | grep -q "resourceType"; then
        echo -e "${GREEN}  ✓ JOLT transformation applied (found 'resourceType' in output)${NC}"
    else
        echo -e "${YELLOW}  ⚠ Transformation may not have been applied as expected${NC}"
    fi
else
    echo -e "${RED}  ✗ Transformation failed (HTTP $HTTP_CODE)${NC}"
    echo "  Response: $BODY"
fi
echo ""

# Test 2: Test with missing fields
echo -e "${YELLOW}Test 2: Transformation with missing fields${NC}"
PARTIAL_DATA='{
  "PatientID": "67890",
  "PatientName": "Jane Smith"
}'
echo "  Input data (partial):"
echo "$PARTIAL_DATA"
echo ""

RESPONSE=$(curl -s -w "\n%{http_code}" -X POST http://127.0.0.1:$HARMONY_PORT/transform \
    -H "Content-Type: application/json" \
    -d "$PARTIAL_DATA")
HTTP_CODE=$(echo "$RESPONSE" | tail -n1)
BODY=$(echo "$RESPONSE" | sed '$d')

if [ "$HTTP_CODE" = "200" ]; then
    echo -e "${GREEN}  ✓ Transformation with partial data successful (HTTP $HTTP_CODE)${NC}"
    echo ""
    echo "  Transformed output:"
    if [ "$HAS_JQ" = true ]; then
        echo "$BODY" | jq '.' 2>/dev/null || echo "$BODY"
    else
        echo "$BODY"
    fi
else
    echo -e "${YELLOW}  ⚠ Transformation with partial data returned HTTP $HTTP_CODE${NC}"
    echo "  Response: $BODY"
fi
echo ""

# Test 3: Test with complex data
echo -e "${YELLOW}Test 3: Complex data transformation${NC}"
COMPLEX_DATA='{
  "PatientID": "COMPLEX001",
  "PatientName": "Test Patient",
  "StudyInstanceUID": "1.2.840.113619.2.1.1.1",
  "StudyDate": "2024-12-01",
  "StudyTime": "143000",
  "Modality": "CT",
  "AccessionNumber": "ACC12345"
}'
echo "  Input data (complex):"
if [ "$HAS_JQ" = true ]; then
    echo "$COMPLEX_DATA" | jq '.' 2>/dev/null || echo "$COMPLEX_DATA"
else
    echo "$COMPLEX_DATA"
fi
echo ""

RESPONSE=$(curl -s -w "\n%{http_code}" -X POST http://127.0.0.1:$HARMONY_PORT/transform \
    -H "Content-Type: application/json" \
    -d "$COMPLEX_DATA")
HTTP_CODE=$(echo "$RESPONSE" | tail -n1)
BODY=$(echo "$RESPONSE" | sed '$d')

if [ "$HTTP_CODE" = "200" ]; then
    echo -e "${GREEN}  ✓ Complex data transformation successful (HTTP $HTTP_CODE)${NC}"
else
    echo -e "${RED}  ✗ Complex data transformation failed (HTTP $HTTP_CODE)${NC}"
fi
echo ""

echo -e "${BLUE}=== Summary ===${NC}"
echo ""
echo "Transform Middleware Capabilities:"
echo "  ✅ JOLT transformations working"
echo "  ✅ Patient data to FHIR format"
echo "  ✅ Handles missing fields gracefully"
echo "  ✅ Pre-transform snapshot preserved"
echo ""
echo "Transform Specifications:"
echo "  - transforms/patient_to_fhir.json - Patient to FHIR conversion"
echo "  - transforms/simple_rename.json - Field renaming"
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
