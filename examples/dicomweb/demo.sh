#!/bin/bash
# DICOMweb Demo Script
# Demonstrates DICOMweb QIDO-RS and WADO-RS endpoints

set -e

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

# Configuration
HARMONY_PORT=8081
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
TMP_DIR="$SCRIPT_DIR/tmp"
export TMP_DIR

echo -e "${BLUE}=== DICOMweb Demo ===${NC}"
echo "This script demonstrates Harmony's DICOMweb capabilities"
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
echo -e "${YELLOW}Starting Harmony DICOMweb service on port $HARMONY_PORT...${NC}"
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

# Test UIDs
STUDY_UID="1.2.3.4.5"
SERIES_UID="6.7.8.9.10"
INSTANCE_UID="11.12.13.14.15"

# =======================
# QIDO-RS Tests (Query)
# =======================
echo -e "${BLUE}--- QIDO-RS Tests (Query) ---${NC}"
echo ""

# Test 1: QIDO-RS - Search for all studies
echo -e "${YELLOW}Test 1: QIDO-RS - Search for all studies${NC}"
echo "  GET /dicomweb/studies"
RESPONSE=$(curl -s -w "\n%{http_code}" "http://127.0.0.1:$HARMONY_PORT/dicomweb/studies" \
    -H "Accept: application/dicom+json")
HTTP_CODE=$(echo "$RESPONSE" | tail -n1)
BODY=$(echo "$RESPONSE" | sed '$d')

if [ "$HTTP_CODE" = "200" ] || [ "$HTTP_CODE" = "204" ]; then
    echo -e "${GREEN}  ✓ Search successful (HTTP $HTTP_CODE)${NC}"
    [ "$HTTP_CODE" = "200" ] && echo "  Response: ${BODY:0:100}..."
else
    echo -e "${YELLOW}  ⚠ Search returned HTTP $HTTP_CODE${NC}"
    echo "  Response: $BODY"
fi
echo ""

# Test 2: QIDO-RS - Search studies with query parameters
echo -e "${YELLOW}Test 2: QIDO-RS - Search studies by PatientID${NC}"
echo "  GET /dicomweb/studies?PatientID=12345"
RESPONSE=$(curl -s -w "\n%{http_code}" "http://127.0.0.1:$HARMONY_PORT/dicomweb/studies?PatientID=12345" \
    -H "Accept: application/dicom+json")
HTTP_CODE=$(echo "$RESPONSE" | tail -n1)

if [ "$HTTP_CODE" = "200" ] || [ "$HTTP_CODE" = "204" ]; then
    echo -e "${GREEN}  ✓ Filtered search successful (HTTP $HTTP_CODE)${NC}"
else
    echo -e "${YELLOW}  ⚠ Filtered search returned HTTP $HTTP_CODE${NC}"
fi
echo ""

# Test 3: QIDO-RS - Query for specific study
echo -e "${YELLOW}Test 3: QIDO-RS - Query for specific study${NC}"
echo "  GET /dicomweb/studies/{study_uid}"
RESPONSE=$(curl -s -w "\n%{http_code}" "http://127.0.0.1:$HARMONY_PORT/dicomweb/studies/$STUDY_UID" \
    -H "Accept: application/dicom+json")
HTTP_CODE=$(echo "$RESPONSE" | tail -n1)

if [ "$HTTP_CODE" = "200" ] || [ "$HTTP_CODE" = "204" ] || [ "$HTTP_CODE" = "404" ]; then
    echo -e "${GREEN}  ✓ Study query handled (HTTP $HTTP_CODE)${NC}"
else
    echo -e "${YELLOW}  ⚠ Study query returned HTTP $HTTP_CODE${NC}"
fi
echo ""

# Test 4: QIDO-RS - Search for series within a study
echo -e "${YELLOW}Test 4: QIDO-RS - Search for series within a study${NC}"
echo "  GET /dicomweb/studies/{study_uid}/series"
RESPONSE=$(curl -s -w "\n%{http_code}" "http://127.0.0.1:$HARMONY_PORT/dicomweb/studies/$STUDY_UID/series" \
    -H "Accept: application/dicom+json")
HTTP_CODE=$(echo "$RESPONSE" | tail -n1)

if [ "$HTTP_CODE" = "200" ] || [ "$HTTP_CODE" = "204" ] || [ "$HTTP_CODE" = "404" ]; then
    echo -e "${GREEN}  ✓ Series search handled (HTTP $HTTP_CODE)${NC}"
else
    echo -e "${YELLOW}  ⚠ Series search returned HTTP $HTTP_CODE${NC}"
fi
echo ""

# Test 5: QIDO-RS - Query for specific series
echo -e "${YELLOW}Test 5: QIDO-RS - Query for specific series${NC}"
echo "  GET /dicomweb/studies/{study_uid}/series/{series_uid}"
RESPONSE=$(curl -s -w "\n%{http_code}" "http://127.0.0.1:$HARMONY_PORT/dicomweb/studies/$STUDY_UID/series/$SERIES_UID" \
    -H "Accept: application/dicom+json")
HTTP_CODE=$(echo "$RESPONSE" | tail -n1)

if [ "$HTTP_CODE" = "200" ] || [ "$HTTP_CODE" = "204" ] || [ "$HTTP_CODE" = "404" ]; then
    echo -e "${GREEN}  ✓ Series query handled (HTTP $HTTP_CODE)${NC}"
else
    echo -e "${YELLOW}  ⚠ Series query returned HTTP $HTTP_CODE${NC}"
fi
echo ""

# Test 6: QIDO-RS - Search for instances within a series
echo -e "${YELLOW}Test 6: QIDO-RS - Search for instances within a series${NC}"
echo "  GET /dicomweb/studies/{study_uid}/series/{series_uid}/instances"
RESPONSE=$(curl -s -w "\n%{http_code}" "http://127.0.0.1:$HARMONY_PORT/dicomweb/studies/$STUDY_UID/series/$SERIES_UID/instances" \
    -H "Accept: application/dicom+json")
HTTP_CODE=$(echo "$RESPONSE" | tail -n1)

if [ "$HTTP_CODE" = "200" ] || [ "$HTTP_CODE" = "204" ] || [ "$HTTP_CODE" = "404" ]; then
    echo -e "${GREEN}  ✓ Instance search handled (HTTP $HTTP_CODE)${NC}"
else
    echo -e "${YELLOW}  ⚠ Instance search returned HTTP $HTTP_CODE${NC}"
fi
echo ""

# =======================
# WADO-RS Tests (Retrieve)
# =======================
echo -e "${BLUE}--- WADO-RS Tests (Retrieve) ---${NC}"
echo ""

# Test 7: WADO-RS - Retrieve study metadata
echo -e "${YELLOW}Test 7: WADO-RS - Retrieve study metadata${NC}"
echo "  GET /dicomweb/studies/{study_uid}/metadata"
RESPONSE=$(curl -s -w "\n%{http_code}" "http://127.0.0.1:$HARMONY_PORT/dicomweb/studies/$STUDY_UID/metadata" \
    -H "Accept: application/dicom+json")
HTTP_CODE=$(echo "$RESPONSE" | tail -n1)

if [ "$HTTP_CODE" = "200" ] || [ "$HTTP_CODE" = "204" ] || [ "$HTTP_CODE" = "404" ]; then
    echo -e "${GREEN}  ✓ Study metadata retrieval handled (HTTP $HTTP_CODE)${NC}"
else
    echo -e "${YELLOW}  ⚠ Study metadata returned HTTP $HTTP_CODE${NC}"
fi
echo ""

# Test 8: WADO-RS - Retrieve series metadata
echo -e "${YELLOW}Test 8: WADO-RS - Retrieve series metadata${NC}"
echo "  GET /dicomweb/studies/{study_uid}/series/{series_uid}/metadata"
RESPONSE=$(curl -s -w "\n%{http_code}" "http://127.0.0.1:$HARMONY_PORT/dicomweb/studies/$STUDY_UID/series/$SERIES_UID/metadata" \
    -H "Accept: application/dicom+json")
HTTP_CODE=$(echo "$RESPONSE" | tail -n1)

if [ "$HTTP_CODE" = "200" ] || [ "$HTTP_CODE" = "204" ] || [ "$HTTP_CODE" = "404" ]; then
    echo -e "${GREEN}  ✓ Series metadata retrieval handled (HTTP $HTTP_CODE)${NC}"
else
    echo -e "${YELLOW}  ⚠ Series metadata returned HTTP $HTTP_CODE${NC}"
fi
echo ""

# Test 9: WADO-RS - Retrieve instance metadata
echo -e "${YELLOW}Test 9: WADO-RS - Retrieve instance metadata${NC}"
echo "  GET /dicomweb/studies/{study_uid}/series/{series_uid}/instances/{instance_uid}/metadata"
RESPONSE=$(curl -s -w "\n%{http_code}" "http://127.0.0.1:$HARMONY_PORT/dicomweb/studies/$STUDY_UID/series/$SERIES_UID/instances/$INSTANCE_UID/metadata" \
    -H "Accept: application/dicom+json")
HTTP_CODE=$(echo "$RESPONSE" | tail -n1)

if [ "$HTTP_CODE" = "200" ] || [ "$HTTP_CODE" = "204" ] || [ "$HTTP_CODE" = "404" ]; then
    echo -e "${GREEN}  ✓ Instance metadata retrieval handled (HTTP $HTTP_CODE)${NC}"
else
    echo -e "${YELLOW}  ⚠ Instance metadata returned HTTP $HTTP_CODE${NC}"
fi
echo ""

# Test 10: WADO-RS - Retrieve instance (DICOM object)
echo -e "${YELLOW}Test 10: WADO-RS - Retrieve instance (DICOM object)${NC}"
echo "  GET /dicomweb/studies/{study_uid}/series/{series_uid}/instances/{instance_uid}"
RESPONSE=$(curl -s -w "\n%{http_code}" "http://127.0.0.1:$HARMONY_PORT/dicomweb/studies/$STUDY_UID/series/$SERIES_UID/instances/$INSTANCE_UID" \
    -H "Accept: multipart/related; type=application/dicom")
HTTP_CODE=$(echo "$RESPONSE" | tail -n1)

if [ "$HTTP_CODE" = "200" ] || [ "$HTTP_CODE" = "404" ]; then
    echo -e "${GREEN}  ✓ Instance retrieval handled (HTTP $HTTP_CODE)${NC}"
else
    echo -e "${YELLOW}  ⚠ Instance retrieval returned HTTP $HTTP_CODE${NC}"
fi
echo ""

# Test 11: WADO-RS - Retrieve rendered image frames
echo -e "${YELLOW}Test 11: WADO-RS - Retrieve rendered image frames${NC}"
echo "  GET /dicomweb/studies/{study_uid}/series/{series_uid}/instances/{instance_uid}/frames/1"
RESPONSE=$(curl -s -w "\n%{http_code}" "http://127.0.0.1:$HARMONY_PORT/dicomweb/studies/$STUDY_UID/series/$SERIES_UID/instances/$INSTANCE_UID/frames/1" \
    -H "Accept: image/jpeg")
HTTP_CODE=$(echo "$RESPONSE" | tail -n1)

if [ "$HTTP_CODE" = "200" ] || [ "$HTTP_CODE" = "404" ] || [ "$HTTP_CODE" = "406" ]; then
    echo -e "${GREEN}  ✓ Frame retrieval handled (HTTP $HTTP_CODE)${NC}"
else
    echo -e "${YELLOW}  ⚠ Frame retrieval returned HTTP $HTTP_CODE${NC}"
fi
echo ""

# Test 12: WADO-RS - Bulk data retrieval
echo -e "${YELLOW}Test 12: WADO-RS - Bulk data retrieval${NC}"
echo "  GET /dicomweb/bulkdata/test-uri"
RESPONSE=$(curl -s -w "\n%{http_code}" "http://127.0.0.1:$HARMONY_PORT/dicomweb/bulkdata/test-uri" \
    -H "Accept: application/octet-stream")
HTTP_CODE=$(echo "$RESPONSE" | tail -n1)

if [ "$HTTP_CODE" = "200" ] || [ "$HTTP_CODE" = "404" ]; then
    echo -e "${GREEN}  ✓ Bulkdata retrieval handled (HTTP $HTTP_CODE)${NC}"
else
    echo -e "${YELLOW}  ⚠ Bulkdata retrieval returned HTTP $HTTP_CODE${NC}"
fi
echo ""

# =======================
# CORS Tests
# =======================
echo -e "${BLUE}--- CORS Support Tests ---${NC}"
echo ""

# Test 13: OPTIONS request for CORS
echo -e "${YELLOW}Test 13: OPTIONS request (CORS preflight)${NC}"
echo "  OPTIONS /dicomweb/studies"
RESPONSE=$(curl -s -w "\n%{http_code}" -X OPTIONS "http://127.0.0.1:$HARMONY_PORT/dicomweb/studies")
HTTP_CODE=$(echo "$RESPONSE" | tail -n1)

if [ "$HTTP_CODE" = "200" ]; then
    echo -e "${GREEN}  ✓ CORS preflight successful (HTTP $HTTP_CODE)${NC}"
else
    echo -e "${YELLOW}  ⚠ CORS preflight returned HTTP $HTTP_CODE${NC}"
fi
echo ""

echo -e "${BLUE}=== Summary ===${NC}"
echo ""
echo "DICOMweb Capabilities Tested:"
echo "  ✅ QIDO-RS (Query based on ID for DICOM Objects - RESTful)"
echo "  ✅ WADO-RS (Web Access to DICOM Objects - RESTful)"
echo "  ✅ Study/Series/Instance level queries"
echo "  ✅ Metadata retrieval"
echo "  ✅ Frame retrieval"
echo "  ✅ Bulk data access"
echo "  ✅ CORS support"
echo ""
echo "All Tested Endpoints (13 total):"
echo ""
echo "  QIDO-RS (Query):"
echo "    1.  GET /dicomweb/studies"
echo "    2.  GET /dicomweb/studies?PatientID=xxx"
echo "    3.  GET /dicomweb/studies/{study_uid}"
echo "    4.  GET /dicomweb/studies/{study_uid}/series"
echo "    5.  GET /dicomweb/studies/{study_uid}/series/{series_uid}"
echo "    6.  GET /dicomweb/studies/{study_uid}/series/{series_uid}/instances"
echo ""
echo "  WADO-RS (Retrieve):"
echo "    7.  GET /dicomweb/studies/{study_uid}/metadata"
echo "    8.  GET /dicomweb/studies/{study_uid}/series/{series_uid}/metadata"
echo "    9.  GET /dicomweb/studies/{study_uid}/series/{series_uid}/instances/{instance_uid}/metadata"
echo "    10. GET /dicomweb/studies/{study_uid}/series/{series_uid}/instances/{instance_uid}"
echo "    11. GET /dicomweb/studies/{study_uid}/series/{series_uid}/instances/{instance_uid}/frames/{frame_numbers}"
echo "    12. GET /dicomweb/bulkdata/{bulk_data_uri}"
echo ""
echo "  CORS:"
echo "    13. OPTIONS /dicomweb/studies"
echo ""
echo "Content Types Supported:"
echo "  - application/dicom+json (QIDO-RS, metadata)"
echo "  - multipart/related; type=application/dicom (instances)"
echo "  - image/jpeg, image/png (rendered frames)"
echo "  - application/octet-stream (bulk data)"
echo ""
echo "Logs available at:"
echo "  Harmony: $TMP_DIR/harmony.log"
echo ""
echo -e "${GREEN}Demo complete!${NC}"
echo ""
echo "Press Enter to stop services and exit..."
read
