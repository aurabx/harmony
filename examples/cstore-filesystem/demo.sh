#!/bin/bash
# C-STORE Filesystem Demo Script
# Starts Harmony with DICOM SCP and demonstrates C-STORE to filesystem backend

set -e

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

# Configuration
HARMONY_PORT=8080
DIMSE_PORT=11112
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
TMP_DIR="$SCRIPT_DIR/tmp"
DATA_DIR="$SCRIPT_DIR/data"
export TMP_DIR

echo -e "${BLUE}=== C-STORE Filesystem Demo ===${NC}"
echo "This script demonstrates receiving DICOM files via C-STORE and saving them to disk"
echo ""

# Check for required tools
echo -e "${YELLOW}Checking prerequisites...${NC}"
if ! command -v storescu &> /dev/null; then
    echo -e "${RED}Error: storescu (DCMTK) not found. Please install dcmtk.${NC}"
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
mkdir -p "$DATA_DIR"
# Generate a dummy DICOM file for testing
# We use a sample file from the repo if available
TEST_DCM="$TMP_DIR/test.dcm"
SAMPLE_DCM="$PROJECT_ROOT/samples/dicom/study_1/series_1/CT.1.1.dcm"

if [ ! -f "$TEST_DCM" ]; then
    echo "Preparing test DICOM file..."
    if [ -f "$SAMPLE_DCM" ]; then
        echo "Using sample file: $SAMPLE_DCM"
        cp "$SAMPLE_DCM" "$TEST_DCM"
    else
        # Fallback: Try to generate if img2dcm is available (and we don't have sample)
        if command -v img2dcm &> /dev/null; then
            echo "Creating dummy DICOM file using img2dcm..."
            # Create a dummy image
            echo "P1" > "$TMP_DIR/test.pbm"
            echo "64 64" >> "$TMP_DIR/test.pbm"
            for i in {1..4096}; do echo -n "1 " >> "$TMP_DIR/test.pbm"; done
            # Attempt conversion - use -i BMP if PBM fails or default checks
            if ! img2dcm -i BMP "$TMP_DIR/test.pbm" "$TEST_DCM" 2>/dev/null; then
                 # Try default input format
                 img2dcm "$TMP_DIR/test.pbm" "$TEST_DCM"
            fi
        fi
    fi
    
    if [ ! -f "$TEST_DCM" ]; then
        echo -e "${YELLOW}Warning: Could not create or find test DICOM file.${NC}"
        echo "Please provide a valid DICOM file at $TEST_DCM to run the store test."
    fi
fi

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
echo -e "${YELLOW}Starting Harmony C-STORE service...${NC}"
cd "$SCRIPT_DIR"
"$PROJECT_ROOT/target/release/harmony" --config ./config.toml > "$TMP_DIR/harmony.log" 2>&1 &
HARMONY_PID=$!
echo -e "${GREEN}✓ Harmony started (PID: $HARMONY_PID)${NC}"

# Wait for Harmony to be ready
echo -e "${YELLOW}Waiting for Harmony to be ready...${NC}"
# We check the management port (implied or explicit) or wait for log message
# Simple wait loop checking if port 11112 is open
for i in {1..30}; do
    if nc -z 127.0.0.1 $DIMSE_PORT 2>/dev/null; then
        echo -e "${GREEN}✓ Harmony DIMSE listener is ready on port $DIMSE_PORT${NC}"
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

if [ -f "$TEST_DCM" ]; then
    # Test 1: C-STORE
    echo -e "${YELLOW}Test 1: C-STORE Request${NC}"
    echo "  Command: storescu -v -aet TEST_SCU -aec HARMONY_SCP localhost $DIMSE_PORT $TEST_DCM"
    
    if storescu -v -aet TEST_SCU -aec HARMONY_SCP localhost $DIMSE_PORT "$TEST_DCM" >> "$TMP_DIR/storescu.log" 2>&1; then
        echo -e "${GREEN}  ✓ C-STORE successful${NC}"
        
        # Verify file exists in data dir
        echo -e "${YELLOW}Verifying stored file...${NC}"
        # Look for any .dcm file in data/archive/dimse (recursive)
        FOUND_FILES=$(find "$DATA_DIR" -name "*.dcm")
        
        if [ ! -z "$FOUND_FILES" ]; then
             echo -e "${GREEN}  ✓ Found stored file(s):${NC}"
             echo "$FOUND_FILES"
        else
             echo -e "${RED}  ✗ File not found in storage directory ($DATA_DIR)${NC}"
             find "$DATA_DIR"
        fi
    else
        echo -e "${RED}  ✗ C-STORE failed${NC}"
        cat "$TMP_DIR/storescu.log"
    fi
else
    echo -e "${YELLOW}Skipping C-STORE test (no test.dcm found)${NC}"
fi

echo ""
echo -e "${BLUE}=== Summary ===${NC}"
echo ""
echo "Logs available at:"
echo "  Harmony: $TMP_DIR/harmony.log"
echo "  StoreSCU: $TMP_DIR/storescu.log"
echo ""
echo -e "${GREEN}Demo complete!${NC}"
echo ""
echo "Press Enter to stop services and exit..."
read
